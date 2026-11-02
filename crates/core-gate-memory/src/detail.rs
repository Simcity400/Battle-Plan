use serde::{Serialize, de::DeserializeOwned, ser::SerializeSeq};
use std::{
    fs::File,
    io::{self, BufReader, BufWriter, Write},
    marker::PhantomData,
    path::{Path, PathBuf},
};

pub struct Detail<T> {
    pub path: PathBuf,
    pub slots: usize,
    marker: PhantomData<T>,
}

impl<T> Detail<T> {
    pub fn new(path: &Path, slots: usize) -> io::Result<Self> {
        std::fs::create_dir(path)?;
        Ok(Self {
            path: path.into(),
            slots,
            marker: PhantomData,
        })
    }

    pub fn write(&self, index: usize, value: Option<&T>) -> io::Result<()>
    where
        T: Serialize,
    {
        let mut out = BufWriter::new(File::create_new(
            self.path.join(format!("{index:08}.json")),
        )?);
        serde_json::to_writer(&mut out, &value)?;
        out.flush()
    }

    pub fn replace(&self, index: usize, value: Option<&T>) -> io::Result<()>
    where
        T: Serialize,
    {
        let mut out = BufWriter::new(
            File::options()
                .write(true)
                .truncate(true)
                .open(self.path.join(format!("{index:08}.json")))?,
        );
        serde_json::to_writer(&mut out, &value)?;
        out.flush()
    }

    pub fn read(&self, index: usize) -> io::Result<Option<T>>
    where
        T: DeserializeOwned,
    {
        Ok(serde_json::from_reader(BufReader::new(File::open(
            self.path.join(format!("{index:08}.json")),
        )?))?)
    }

    pub fn for_each(&self, mut visit: impl FnMut(T) -> io::Result<()>) -> io::Result<()>
    where
        T: DeserializeOwned,
    {
        for index in 0..self.slots {
            if let Some(value) = self.read(index)? {
                visit(value)?;
            }
        }
        Ok(())
    }
}

impl<T: Serialize + DeserializeOwned> Serialize for Detail<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(None)?;
        for index in 0..self.slots {
            if let Some(value) = self.read(index).map_err(serde::ser::Error::custom)? {
                seq.serialize_element(&value)?;
            }
        }
        seq.end()
    }
}
