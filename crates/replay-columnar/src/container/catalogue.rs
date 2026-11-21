
use super::{ContainerWriter, SectionKey, SectionKind, Tags};
use crate::{
    envelope::Kind,
    invalid,
    leaf::{Reader, Writer},
};
use std::{
    collections::HashMap,
    io::{self, Seek, Write},
};

#[derive(Default)]
pub struct Catalogue {
    keys: Vec<Box<[u8]>>,
    ids: HashMap<Box<[u8]>, u32, foldhash::fast::RandomState>,
    written: usize,
}

impl Catalogue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub fn id(&self, key: &[u8]) -> Option<u32> {
        self.ids.get(key).copied()
    }

    pub fn key(&self, id: u32) -> Option<&[u8]> {
        self.keys.get(id as usize).map(|key| &**key)
    }

    pub fn insert(&mut self, key: &[u8]) -> io::Result<u32> {
        if let Some(id) = self.id(key) {
            return Ok(id);
        }
        let id = u32::try_from(self.keys.len()).map_err(|_| invalid("A catalogue passes 2^32 ids"))?;
        let owned: Box<[u8]> = engine_allocation::copy_vec(key)?.into_boxed_slice();
        engine_allocation::reserve(&mut self.keys, 1)?;
        engine_allocation::reserve_map(&mut self.ids, 1)?;
        self.ids.insert(owned.clone(), id);
        self.keys.push(owned);
        Ok(id)
    }

    pub fn pending(&self) -> io::Result<Option<Vec<u8>>> {
        if self.written == self.keys.len() {
            return Ok(None);
        }
        let mut writer = Writer::new();
        writer.varint(self.written as u64)?;
        writer.varint((self.keys.len() - self.written) as u64)?;
        for key in &self.keys[self.written..] {
            writer.varint(key.len() as u64)?;
            writer.raw(key)?;
        }
        Ok(Some(writer.into_bytes()))
    }

    pub fn write<W: Write + Seek>(&mut self, writer: &mut ContainerWriter<W>, id: u32, level: i32) -> io::Result<()> {
        let Some(chunk) = self.pending()? else {
            return Ok(());
        };
        let key = SectionKey::new(SectionKind::Catalogue, id);
        let extent = writer.put_zstd(Kind::Data, Tags::default(), &chunk, level)?;
        writer.append(key, extent)?;
        self.written = self.keys.len();
        Ok(())
    }

    pub fn read_chunk(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut reader = Reader::new(bytes);
        if reader.count()? != self.keys.len() {
            return Err(invalid("A catalogue chunk does not follow the chunk before it"));
        }
        let count = reader.count()?;
        if count > bytes.len() {
            return Err(invalid("A catalogue chunk counts more keys than it holds"));
        }
        for _ in 0..count {
            let length = reader.count()?;
            let key = reader.raw(length)?;
            if self.id(key).is_some() {
                return Err(invalid("A catalogue gives one key two ids"));
            }
            self.insert(key)?;
        }
        reader.finish()?;
        self.written = self.keys.len();
        Ok(())
    }
}
