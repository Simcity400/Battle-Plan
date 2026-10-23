use super::*;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io::Write};

product_codec::product! {
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Key {
        pub recording: [u8; 32], pub anchor: [u8; 32], pub inputs: [u8; 32],
        pub content: [u8; 32], pub content_entries: u64, pub variant: Variant,
        pub producers: BTreeMap<u16, Producer>, pub schema: [u8; 32],
    }
    #[derive(Clone)]
    pub(super) struct Binding { pub key: Key, pub prefix_length: u64, pub prefix_digest: [u8; 32] }
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
        let mut producers = std::collections::BTreeMap::new();
        producers.insert(
            SNOW,
            Producer {
                contract: SNOW_CONTRACT.to_vec(),
                version,
                schema: product_codec::schema_digest::<SnowMud>(),
                variant: Vec::new(),
            },
        );
        Ok(Self {
            recording: [0; 32],
            anchor: [0; 32],
            inputs: identity.0.finalize().into(),
            content: Witness::default().digest,
            content_entries: 0,
            variant: Variant::snow(),
            producers,
            schema: schema_digest(),
        })
    }
    pub fn for_loaded_inputs(inputs: &Inputs, version: u32) -> io::Result<Self> {
        let mut key = Self::for_inputs(Ok((&inputs.cells, &inputs.winter)), version)?;
        key.content = inputs.witness.digest;
        key.content_entries = inputs.witness.entries;
        Ok(key)
    }
    pub fn bind_recording(
        mut self,
        replay: &Replay,
        check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<Self> {
        self.recording = witness(replay, 0, None, check)?.0;
        self.anchor = self.recording;
        Ok(self)
    }
    pub fn bind_store_recording(
        mut self,
        replay: &Replay,
        store: &Store,
        mut check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<Self> {
        let old = match store.saved_key()? {
            Some(source) => saved_binding(store, &source)?,
            None => None,
        };
        let (recording, _, proof) = witness(
            replay,
            0,
            old.as_ref().map(|binding| binding.prefix_length),
            &mut check,
        )?;
        self.recording = recording;
        self.anchor = old
            .filter(|old| Some(old.prefix_digest) == proof)
            .map_or(recording, |old| old.key.anchor);
        check()?;
        Ok(self)
    }
    pub fn bytes(&self) -> Vec<u8> {
        product_codec::encode(self).expect("map export key allocation failed")
    }
    pub fn digest(&self) -> [u8; 32] {
        let mut identity = HashWriter(Sha256::new());
        identity.write_all(CONTRACT).expect("hash writer");
        let mut common = self.clone();
        common.recording = [0; 32];
        identity.write_all(&common.bytes()).expect("hash writer");
        identity.0.finalize().into()
    }
    pub fn layer_digest(&self, layer: u16) -> io::Result<[u8; 32]> {
        let producer = self
            .producers
            .get(&layer)
            .ok_or_else(|| invalid("map export layer has no producer contract"))?;
        let mut hash = Sha256::new();
        hash.update(self.digest());
        hash.update(layer.to_le_bytes());
        hash.update(product_codec::encode(producer)?);
        Ok(hash.finalize().into())
    }
    pub fn table_digest(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(b"map-export/operand-tables/v2");
        hash.update(self.anchor);
        hash.finalize().into()
    }
    pub fn section(&self, name: &str) -> String {
        format!("map-export-v{FORMAT}/{name}")
    }
    pub fn records(&self, branch: BranchId) -> String {
        self.section(&format!("endpoints/{branch}"))
    }
}

impl Binding {
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::from(CONTRACT);
        bytes.extend_from_slice(
            &product_codec::encode(self).expect("map export binding allocation failed"),
        );
        bytes.extend_from_slice(&codec::digest(&bytes));
        bytes
    }
    pub fn read(range: store::Range) -> io::Result<Self> {
        let length = usize::try_from(range.len())
            .map_err(|_| invalid("map export binding exceeds address space"))?;
        let bytes = codec::read_exact_range(range, length)?;
        if bytes.len() < CONTRACT.len() + 32 {
            return Err(invalid("short map export binding"));
        }
        let end = bytes.len() - 32;
        if codec::digest(&bytes[..end]) != bytes[end..] || &bytes[..CONTRACT.len()] != CONTRACT {
            return Err(invalid(
                "map export binding checksum or producer contract differs",
            ));
        }
        let binding: Self = product_codec::decode(&bytes[CONTRACT.len()..end])?;
        if binding.key.schema != schema_digest() {
            return Err(invalid("map export codec schema differs"));
        }
        Ok(binding)
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
        return Err(invalid("map export recording prefix is truncated"));
    }
    Ok((
        full.finalize().into(),
        current.finalize().into(),
        previous
            .filter(|&n| n <= offset)
            .map(|_| old.finalize().into()),
    ))
}
