use super::*;
use replay::container::Replay;
use sha2::{Digest, Sha256};
use std::io::Write;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Key {
    pub recording: [u8; 32],
    pub inputs: [u8; 32],
    pub version: u32,
}

struct HashWriter(Sha256);
impl Write for HashWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Key {
    pub fn for_inputs(
        inputs: Result<(&SnowCells, &Winter), &Status>,
        version: u32,
    ) -> io::Result<Self> {
        let mut identity = HashWriter(Sha256::new());
        match inputs {
            Ok((cells, winter)) => {
                identity.write_all(&[0])?;
                cells.write_identity(&mut identity)?;
                for value in winter
                    .defines
                    .per_province
                    .into_iter()
                    .chain(winter.defines.total)
                {
                    identity.write_all(&value.to_bits().to_le_bytes())?;
                }
                identity.write_all(&(winter.records.len() as u64).to_le_bytes())?;
                for records in &winter.records {
                    identity.write_all(&(records.len() as u64).to_le_bytes())?;
                    for record in records {
                        identity.write_all(&record.province.to_le_bytes())?;
                        identity.write_all(&record.weight.to_bits().to_le_bytes())?;
                    }
                }
            }
            Err(Status::Unavailable(reason) | Status::Unsupported(reason)) => {
                identity.write_all(&[if matches!(inputs, Err(Status::Unavailable(_))) {
                    1
                } else {
                    2
                }])?;
                identity.write_all(&(reason.len() as u64).to_le_bytes())?;
                identity.write_all(reason.as_bytes())?;
            }
            Err(Status::Available) => {
                return Err(invalid("snow available identity needs loaded inputs"));
            }
        }
        Ok(Self {
            recording: [0; 32],
            inputs: identity.0.finalize().into(),
            version,
        })
    }
    pub fn bind_recording(
        mut self,
        replay: &Replay,
        check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<Self> {
        self.recording = witness(replay, 0, None, check)?.0;
        Ok(self)
    }
    pub fn bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::from(CONTRACT);
        bytes.extend_from_slice(&self.version.to_le_bytes());
        bytes.extend_from_slice(&self.recording);
        bytes.extend_from_slice(&self.inputs);
        bytes
    }
    pub fn digest(&self) -> [u8; 32] {
        let mut bytes = Vec::from(CONTRACT);
        bytes.extend_from_slice(&self.version.to_le_bytes());
        bytes.extend_from_slice(&self.inputs);
        codec::digest(&bytes)
    }
    pub fn section(&self, name: &str) -> String {
        format!("map-export-snow-v1/{name}")
    }
    pub fn records(&self, branch: BranchId) -> String {
        self.section(&format!("endpoints/{branch}"))
    }
}

#[derive(Clone)]
pub(super) struct Binding {
    pub key: Key,
    pub prefix_length: u64,
    pub prefix_digest: [u8; 32],
}
impl Binding {
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = self.key.bytes();
        bytes.extend_from_slice(&self.prefix_length.to_le_bytes());
        bytes.extend_from_slice(&self.prefix_digest);
        bytes.extend_from_slice(&codec::digest(&bytes));
        bytes
    }
    pub fn read(range: store::Range) -> io::Result<Self> {
        let bytes = codec::read_exact_range(range, CONTRACT.len() + 4 + 64 + 72)?;
        let end = bytes.len() - 32;
        if codec::digest(&bytes[..end]) != bytes[end..] || &bytes[..CONTRACT.len()] != CONTRACT {
            return Err(invalid(
                "snow binding checksum or producer contract differs",
            ));
        }
        let mut d = replay_data::codec::Decoder::new(&bytes[CONTRACT.len()..end]);
        let key = Key {
            version: d.u32()?,
            recording: d.take(32)?.try_into().unwrap(),
            inputs: d.take(32)?.try_into().unwrap(),
        };
        let prefix_length = d.u64()?;
        let prefix_digest = d.take(32)?.try_into().unwrap();
        d.finish()?;
        Ok(Self {
            key,
            prefix_length,
            prefix_digest,
        })
    }
}

pub(super) fn witness(
    replay: &Replay,
    prefix: u64,
    previous: Option<u64>,
    mut check: impl FnMut() -> io::Result<()>,
) -> io::Result<([u8; 32], [u8; 32], Option<[u8; 32]>)> {
    let mut full = Sha256::new();
    let mut current = Sha256::new();
    let mut old = Sha256::new();
    let mut offset = 0;
    check()?;
    replay.visit_held_bytes(|bytes| {
        check()?;
        full.update(bytes);
        let count = prefix.saturating_sub(offset).min(bytes.len() as u64) as usize;
        current.update(&bytes[..count]);
        if let Some(previous) = previous {
            let count = previous.saturating_sub(offset).min(bytes.len() as u64) as usize;
            old.update(&bytes[..count]);
        }
        offset += bytes.len() as u64;
        Ok(())
    })?;
    check()?;
    if prefix > offset {
        return Err(invalid("snow recording prefix is truncated"));
    }
    Ok((
        full.finalize().into(),
        current.finalize().into(),
        previous
            .filter(|&n| n <= offset)
            .map(|_| old.finalize().into()),
    ))
}
