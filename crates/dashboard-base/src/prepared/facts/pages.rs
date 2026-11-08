use super::*;
use std::{
    io::{Read, Write},
    rc::Rc,
};

pub trait ReadPages {
    fn open(&mut self, section: &str, range: u64) -> io::Result<Option<store::Range>>;
}
impl ReadPages for store::Writer {
    fn open(&mut self, section: &str, range: u64) -> io::Result<Option<store::Range>> {
        self.read_at_spooled(section, range)
    }
}
impl ReadPages for store::Session {
    fn open(&mut self, section: &str, range: u64) -> io::Result<Option<store::Range>> {
        self.read(section, range)
    }
}
impl ReadPages for store::Snapshot {
    fn open(&mut self, section: &str, range: u64) -> io::Result<Option<store::Range>> {
        self.read(section, range)
    }
}

pub(crate) trait Page: Serialize + DeserializeOwned + 'static {}
impl<T: Serialize + DeserializeOwned + 'static> Page for T {}
// A page lives while a cursor holds it; nothing is kept between reads.
pub(crate) struct Pages<'a, R> {
    pub source: &'a mut R,
}
impl<'a, R: ReadPages> Pages<'a, R> {
    pub fn new(source: &'a mut R) -> Self {
        Self { source }
    }
    pub fn read<T: Page>(&mut self, section: &str, range: u64) -> io::Result<Rc<T>> {
        let input = self
            .source
            .open(section, range)?
            .ok_or_else(|| invalid("Private fact page missing"))?;
        Ok(Rc::new(serde_json::from_reader(io::BufReader::new(input))?))
    }
}

pub(crate) fn encode<T: Serialize + ?Sized>(value: &T) -> io::Result<Vec<u8>> {
    struct Counter(usize);
    impl Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len())
                .ok_or_else(|| invalid("Fact encoding overflow"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, value)?;
    let mut bytes = Vec::with_capacity(counter.0);
    serde_json::to_writer(&mut bytes, value)?;
    if bytes.len() != counter.0 {
        return Err(invalid("Private fact serialization is not stable"));
    }
    Ok(bytes)
}
pub(crate) fn write(writer: &mut store::Writer, section: &str, data: impl Read) -> io::Result<u64> {
    writer.write_unbuffered(section, data)
}

pub(crate) fn read<T: Page>(
    source: &mut impl ReadPages,
    section: &str,
    range: u64,
) -> io::Result<Rc<T>> {
    Pages::new(source).read(section, range)
}
